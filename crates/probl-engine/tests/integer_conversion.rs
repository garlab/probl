//! Exact conversion at integer boundaries, without changing float inference.
mod common;

use common::*;
use probl_engine::{ErrorKind, Limits, Options, value::Value};
use probl_sema::ir::Mode;

fn options(mode: Mode) -> Options {
    Options {
        mode: Some(mode),
        ..Options::default()
    }
}

fn facts(mode: Mode, source: &str) {
    let out = exec_raw(source, &options(mode)).unwrap_or_else(|e| panic!("{source}\n{e:?}"));
    assert!(!out.reports.is_empty());
    for report in out.reports {
        assert_eq!(report.distribution(), vec![(Value::Bool(true), 1.0)], "{source}");
    }
}

fn rejects(mode: Mode, source: &str, message: &str) {
    let e = exec_raw(source, &options(mode)).expect_err(source);
    assert_eq!(e.kind, ErrorKind::Language, "{source}: {e:?}");
    assert!(e.message.contains(message), "{source}: {e:?}");
}

macro_rules! both_modes {
    ($($test:item)*) => {
        mod enumerate {
            use super::*;
            const MODE: Mode = Mode::Enumerate;
            $($test)*
        }
        mod sample {
            use super::*;
            const MODE: Mode = Mode::Sample { runs: 100, seed: 7 };
            $($test)*
        }
    };
}

both_modes! {
    #[test]
    fn bindings_convert_without_changing_source_or_arithmetic_types() {
        facts(
            MODE,
            r#"
            let x = 1.0
            let n: int = x
            let literal: int = 1.0
            let negative: int = -2.0
            let zero: int = -0.0
            let percentage: int = 100%
            var assigned: int = 0
            assigned = x + 2
            report typeof x == "float" and typeof (x + 1) == "float"
            report typeof n == "int" and n == 1 and literal == 1
            report negative == -2 and zero == 0 and percentage == 1
            report typeof assigned == "int" and assigned == 3
        "#,
        );
    }

    #[test]
    fn parameter_return_record_and_branch_boundaries_agree() {
        facts(
            MODE,
            r#"
            fn argument(n: int) { typeof n == "int" and n == 2 }
            fn result(n) -> int { n }
            fn explicit(n) -> int { return n }
            type Box = { n: int }
            let x = 2.0
            var b = Box { n: x }
            b.n = 3.0
            let c = b with { n: x }
            let a: { n: int } = { n: x }
            let choose: int = if true { 2.0 } else { 3.0 }
            let matched: int = match true { true => 2.0, false => 3.0 }
            let blocked: int = { let temp = x; temp }
            report argument(x) and argument(2.0) and argument(result(x)) and argument(explicit(x))
            report typeof result(x) == "int" and typeof explicit(x) == "int"
            report b.n == 3 and typeof b.n == "int" and c.n == 2 and typeof c.n == "int"
            report typeof a.n == "int" and choose == 2 and typeof choose == "int"
            report typeof matched == "int" and typeof blocked == "int"
        "#,
        );
    }

    #[test]
    fn recursive_container_conversion_preserves_sources_and_merges_bag_counts() {
        facts(
            MODE,
            r#"
            let xs = [[1.0, 2.0]]
            let ys: list[list[int]] = xs
            var zs: list[int] = [1.0, 2.0]
            zs[0.0] = 3.0
            zs.push(4.0)
            let m: map[int, list[int]] = [1.0: [2.0]]
            let b: bag[int] = bag([1, 1.0, 2.0])
            let d: dist[int] = one_of([1, 1.0, 2.0])
            report typeof xs == "list[list[float]]" and typeof ys == "list[list[int]]"
            report zs == [3, 2, 4] and typeof zs == "list[int]"
            report typeof m == "map[int, list[int]]" and m[1] == [2]
            report typeof b == "bag[int]" and len(b) == 3 and b.get(1) == 2
            report typeof d == "dist[int]" and len(support(d)) == 2
            report abs(pmf(d, 1) - 2/3) < 1e-15
        "#,
        );
        for source in [
            "let m: map[int,str] = [1: \"a\", 1.0: \"b\"]; report m",
            "let raw = [1: \"a\", 1.0: \"b\"]; let m: map[int,str] = raw; report m",
            "let raw = [[1]: \"a\", [1.0]: \"b\"]; let m: map[list[int],str] = raw; report m",
        ] {
            rejects(MODE, source, "collision");
        }
    }

    #[test]
    fn float_indices_work_for_reads_writes_and_collection_methods() {
        facts(
            MODE,
            r#"
            let xs = [10,20,30]
            var ys = xs
            ys[1.0] = 40
            ys.insert(3.0,50)
            ys.remove(0.0)
            report xs[1.0] == 20 and xs.get(1.0,-1) == 20 and xs.get(-0.0) == 10
            report ys == [40,30,50] and xs == [10,20,30]
            report slice(xs,1.0,3.0) == [20,30]
            report "a🙂b"[1.0] == "🙂" and slice("a🙂b",1.0,2.0) == "🙂"
            report (10..20)[2.0] == 12 and slice(10..20,2.0,4.0) == (12..13)
            report slice(xs,3.0,3.0) == []
        "#,
        );
        for position in ["-1.0", "3.0", "1e100", "10^400"] {
            facts(MODE, &format!("let xs=[10,20,30]; report xs.get({position},-1) == -1"));
            rejects(MODE, &format!("report [10,20,30][{position}]"), "out of range");
        }
    }

    #[test]
    fn invalid_indices_are_errors_even_with_a_default_or_empty_collection() {
        for value in [
            "1.4",
            "1.0000000000000002",
            "-0.5",
            "5e-324",
            "true",
            "\"1\"",
            "complex(1)",
            "prob(1)",
        ] {
            for expression in [
                "xs[i]",
                "xs.get(i,-1)",
                "[].get(i,-1)",
                "insert(xs,i,0)",
                "remove(xs,i)",
                "slice(xs,i)",
                "slice(xs,0,i)",
                "\"abc\"[i]",
                "(10..20)[i]",
            ] {
                rejects(
                    MODE,
                    &format!("let i={value}; let xs=[10,20]; report {expression}"),
                    "int",
                );
            }
            rejects(
                MODE,
                &format!("var xs=[10,20]; let i={value}; xs[i]=30; report xs"),
                "int",
            );
        }
    }

    #[test]
    fn integer_builtins_counts_ranges_and_dates_share_conversion() {
        facts(
            MODE,
            r#"
            report factorial(5.0) == 120 and choose(5.0,2.0) == 10
            report mean(factorial(one_of([2.0,3.0]))) == 4
            report gcd(12.0,18.0) == 6 and lcm(4.0,6.0) == 12 and euler_phi(9.0) == 6
            report bit_length(8.0) == 4 and ilog2(8.0) == 3 and bit_count(7.0) == 3
            report bit_and(6.0,3.0) == 2 and bit_or(6.0,3.0) == 7
            report bit_xor(6.0,3.0) == 5 and bit_not(0.0) == -1
            report round(1.25,1.0) == 1.3 and round(1250,-2.0) == 1300
            report mean(binomial(2.0,50%)) == 1 and highest([1,2,3],2.0) == [3,2]
            report lowest([1,2,3],2.0) == [1,2] and len(bag(["a":2.0])) == 2
            report mean(len(roll(2.0,1))) == 2
            report mean(sum(roll(1,1.0))) == 1
            report (1.0..3.0) == (1..3) and (1.0..<3.0) == (1..2)
            var count = 0
            repeat 3.0 { count += 1 }
            report count == 3
            let d = date(2026.0,10.0,4.0)
            report d + 1.0 == date("2026-10-05") and 1.0 + d == d + 1
            report d - 1.0 == date("2026-10-03")
            report d.add_months(1.0) == date("2026-11-04")
            report d.add_years(1.0) == date("2027-10-04")
            report d.add_workdays(1.0) == date("2026-10-05")
        "#,
        );
    }

    #[test]
    fn invalid_values_fail_at_all_integer_argument_boundaries() {
        for value in ["1.4", "true", "\"1\"", "complex(1)", "prob(1)"] {
            for expression in [
                "factorial(x)",
                "choose(x,1)",
                "choose(2,x)",
                "gcd(x,2)",
                "lcm(2,x)",
                "euler_phi(x)",
                "bit_length(x)",
                "ilog2(x)",
                "bit_and(2,x)",
                "bit_or(x,2)",
                "bit_xor(2,x)",
                "bit_not(x)",
                "bit_count(x)",
                "round(1.25,x)",
                "binomial(x,50%)",
                "highest([1,2],x)",
                "lowest([1,2],x)",
                "bag([1:x])",
                "roll(x,d1)",
                "date(2026,1,x)",
                "date(2026,1,1).add_months(x)",
                "date(2026,1,1).add_years(x)",
                "date(2026,1,1).add_workdays(x)",
                "(0..x)",
                "(x..<3)",
            ] {
                rejects(MODE, &format!("let x={value}; report {expression}"), "int");
            }
            rejects(MODE, &format!("let x={value}; repeat x {{ }}; report true"), "int");
        }
        rejects(MODE, "report roll(1,1.4)", "int");
        rejects(MODE, "report date(2026,1,1)+1.4", "int");
        rejects(MODE, "report date(2026,1,1)-1.4", "int");
    }

    #[test]
    fn invalid_literals_fail_early_and_computed_values_fail_at_runtime() {
        for source in [
            "let n:int=1.4",
            "let n:int=-1.4",
            "let n:list[int]=[1.4]",
            "fn f(n:int) { n }; report f(1.4)",
            "fn f() -> int { 1.4 }; report f()",
            "let n:int=if true {1.0} else {1.4}",
        ] {
            assert!(compile_error(source).contains("int"), "{source}");
        }
        for value in [
            "1.4",
            "1.0000000000000002",
            "0.9999999999999999",
            "true",
            "\"1\"",
            "complex(1)",
            "prob(1)",
        ] {
            for context in [
                "let n:int=x",
                "var n:int=0; n=x",
                "fn f(n:int) { n }; let n=f(x)",
                "fn f(x) -> int { x }; let n=f(x)",
                "type Box={n:int}; let b=Box {n:x}",
                "let ns:list[int]=[x]",
                "let ns:dist[int]=one_of([x])",
            ] {
                rejects(MODE, &format!("let x={value}; {context}; report true"), "int");
            }
        }
    }

    #[test]
    fn conversion_errors_never_condition_away_invalid_worlds_or_outcomes() {
        rejects(MODE, "let x ~ one_of([1.0,1.4]); let n:int=x; report n", "int");
        rejects(MODE, "let x=one_of([1.0,1.4]); report factorial(x)", "int");
        rejects(MODE, "let n:dist[int]=one_of([1.0,1.4]); report n", "int");
        let out = exec_raw(
            "let x ~ one_of([1.0,2.0]); let n:int=x; report typeof n; report typeof x",
            &options(MODE),
        )
        .unwrap();
        assert_eq!(out.reports[0].distribution(), vec![(Value::str("int"), 1.0)]);
        assert_eq!(out.reports[1].distribution(), vec![(Value::str("float"), 1.0)]);
    }

    #[test]
    fn conversion_keeps_effect_order_and_direct_inference_paths() {
        facts(
            MODE,
            r#"
            var x = 1.0
            fn first(n:int, ignored) { n }
            let n = first(x, { x=2.0; 0 })
            let k ~ binomial(3.0,1)
            let faces ~ roll(2.0,1.0)
            report n == 1 and typeof n == "int" and x == 2.0
            report k == 3 and typeof k == "int" and faces == [1,1]
        "#,
        );
        rejects(MODE, "let k ~ binomial(1.4,50%); report k", "int");
        // Conjugate updates are a sampling feature; enumeration uses the
        // separate, restricted analytic-continuous engine.
        if matches!(MODE, Mode::Sample { .. }) {
            let out = exec_raw(
                "let p ~ beta(1,1); observe 2 from binomial(2.0,p); report typeof p",
                &options(MODE),
            ).unwrap();
            assert!(out.stats.updates.iter().any(|u| u.exact > 0));
            assert_eq!(out.reports[0].distribution(), vec![(Value::str("float"), 1.0)]);
            rejects(
                MODE,
                "let p ~ beta(1,1); observe 1 from binomial(1.4,p); report p",
                "int",
            );
        }
    }

    #[test]
    fn conversion_preserves_large_stored_values_and_enforces_resource_limits() {
        facts(
            MODE,
            r#"
            let x=2.0^100
            let n:int=x
            let exact:int=-(2^200+1)
            let exact_literal:int=-9007199254740993
            let rounded:int=9007199254740993.0
            report n == 2^100 and typeof n == "int" and typeof x == "float"
            report exact == -(2^200+1) and rounded == 9007199254740992
            report exact_literal == -9007199254740993
            report bit_length(x) == 101 and (0..2^101)[x] == 2^100
            report slice(0..2^101,x,x+2.0^50)[0] == 2^100
        "#,
        );
        for source in [
            "let n:int=1e100; report n",
            "let x=1e100; let n:int=x; report n",
            "report bit_length(1e100)",
            "report round(1,1e100)",
            "report [0].get(1e100,-1)",
            "report (1e100..1e100)",
        ] {
            let opts = Options {
                limits: Limits {
                    max_integer_bits: 32,
                    ..Limits::default()
                },
                ..options(MODE)
            };
            let e = exec_raw(source, &opts).expect_err(source);
            assert_eq!(e.kind, ErrorKind::Limit, "{source}: {e:?}");
        }
        let opts = Options {
            limits: Limits {
                max_integer_bytes: 0,
                ..Limits::default()
            },
            ..options(MODE)
        };
        for source in ["let n:int=1e100; report n", "report bit_length(1e100)"] {
            let e = exec_raw(source, &opts).expect_err(source);
            assert_eq!(e.kind, ErrorKind::Limit, "{source}: {e:?}");
        }
    }
}

#[test]
fn host_nonfinite_floats_cannot_enter_integer_contexts() {
    use probl_engine::{dist::Budget, ops};
    let mut budget = Budget::unlimited();
    for value in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
        let e = ops::integer(&Value::Float(value), "test", &mut budget).unwrap_err();
        assert!(e.message.contains("int"));
    }
}
