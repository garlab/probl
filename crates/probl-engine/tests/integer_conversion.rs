//! Exact conversion at integer boundaries, without changing float inference:
//! a declared float takes an int a float holds, a declared int never takes a
//! float, and `int(x)` and `float(x)` convert explicitly.
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
    fn declarations_turn_ints_into_floats_and_never_floats_into_ints() {
        facts(
            MODE,
            r#"
            let n = 1
            let x: float = n
            let literal: float = 1
            let negative: float = -2
            let zero: float = -0
            let large: float = 2^60
            var assigned: float = 0
            assigned = n + 2
            let converted: int = int(2.0)
            report typeof n == "int" and typeof (n + 1) == "int"
            report typeof x == "float" and x == 1 and typeof literal == "float"
            report negative == -2 and typeof negative == "float" and zero == 0
            report large == 2^60 and typeof assigned == "float" and assigned == 3
            report typeof converted == "int" and converted == 2
            report 7 div 2 == 3 and int(4 / 2) == 2 and float(2^53 + 1) == 2^53
        "#,
        );
        // No float holds this int, so no declaration turns it into one.
        for context in ["let x: float = a", "fn f(x: float) { x }; let x = f(a)", "let xs: list[float] = [a]"] {
            rejects(MODE, &format!("let a = 2^53 + 1; {context}; report true"), "no float holds");
        }
    }

    #[test]
    fn int_and_float_convert_explicitly() {
        facts(
            MODE,
            r#"
            report int(2.0) == 2 and typeof int(2.0) == "int" and int(-0.0) == 0 and int(5) == 5
            report typeof float(2) == "float" and float(prob(0.5)) == 0.5 and typeof float(prob(0.5)) == "float"
            report float(2^53 + 1) == 2^53 and typeof float(d6) == "dist[float]"
            report [1, 2].map(float) == [1.0, 2.0] and typeof [1, 2].map(float)[0] == "float"
            let u ~ uniform(0, 10)
            let f: float = floor(u)
            let n: int = floor(u)
            report typeof f == "float" and typeof n == "int"
        "#,
        );
        for value in ["2.5", "prob(1)", "true", "\"42\"", "complex(1)"] {
            rejects(MODE, &format!("report int({value})"), "int");
        }
        for value in ["true", "\"42\"", "complex(1)"] {
            rejects(MODE, &format!("report float({value})"), "float");
        }
        // The errors say what to write instead.
        let e = exec_raw("let x = 4\nlet n: int = x / 2", &options(MODE)).unwrap_err();
        assert!(e.message.contains("`n` should be an int, but it's a float"), "{e:?}");
        assert!(e.help.as_deref().is_some_and(|h| h.contains("`div` divides to an int")), "{e:?}");
        let e = exec_raw("let a = 2^53 + 1\nlet x: float = a", &options(MODE)).unwrap_err();
        assert!(e.message.contains("no float holds 9007199254740993"), "{e:?}");
        assert!(e.help.as_deref().is_some_and(|h| h.contains("`float(…)` rounds")), "{e:?}");
        assert!(compile_error("let n: int = 2.0").contains("write `2`, without the decimal point"));
        assert!(compile_error("let n: int = 2.5").contains("`round(…)`, `floor(…)` and `ceil(…)`"));
        assert!(compile_error("let x: float = 9007199254740993").contains("no float holds 9007199254740993"));
    }

    #[test]
    fn a_named_record_fits_an_anonymous_type_only_as_it_is() {
        facts(
            MODE,
            r#"
            type Point = { x: int }
            let p: { x: int } = Point { x: 1 }
            report typeof p == "Point"
            let r: { x: float } = { x: 1 }
            report typeof r.x == "float"
        "#,
        );
        for source in [
            "type Point = { x: int }; let q: { x: float } = Point { x: 1 }; report q",
            "type Point = { x: int }; let p = Point { x: 1 }; let q: { x: float } = p; report q",
            "type Point = { x: int }; fn f(q: { x: float }) { q }; report f(Point { x: 1 })",
        ] {
            rejects(MODE, source, "wouldn't be a `Point` any more");
        }
    }

    #[test]
    fn parameter_return_record_and_branch_boundaries_agree() {
        facts(
            MODE,
            r#"
            fn argument(x: float) { typeof x == "float" and x == 2 }
            fn result(x): float { x }
            fn explicit(x): float { return x }
            type Box = { x: float }
            let n = 2
            var b = Box { x: n }
            b.x = 3
            let c = b with { x: n }
            let a: { x: float } = { x: n }
            let choose: float = if true { 2 } else { 3 }
            let matched: float = match true { true => 2, false => 3 }
            let blocked: float = { let temp = n; temp }
            report argument(n) and argument(2) and argument(result(n)) and argument(explicit(n))
            report typeof result(n) == "float" and typeof explicit(n) == "float"
            report b.x == 3 and typeof b.x == "float" and c.x == 2 and typeof c.x == "float"
            report typeof a.x == "float" and choose == 2 and typeof choose == "float"
            report typeof matched == "float" and typeof blocked == "float"
        "#,
        );
    }

    #[test]
    fn recursive_container_conversion_preserves_sources_and_merges_bag_counts() {
        facts(
            MODE,
            r#"
            let xs = [[1, 2]]
            let ys: list[list[float]] = xs
            var zs: list[float] = [1, 2]
            zs[0.0] = 3
            zs.push(4)
            let m: map[float, list[float]] = [1: [2]]
            let b: bag[float] = bag([1, 1.0, 2])
            let d: dist[float] = one_of([1, 1.0, 2])
            report typeof xs == "list[list[int]]" and typeof ys == "list[list[float]]"
            report zs == [3, 2, 4] and typeof zs == "list[float]"
            report typeof m == "map[float, list[float]]" and m[1] == [2]
            report typeof b == "bag[float]" and len(b) == 3 and b.get(1) == 2
            report typeof d == "dist[float]" and len(support(d)) == 2
            report abs(pmf(d, 1) - 2/3) < 1e-15
        "#,
        );
        // Keys that are the same number are one key, the first written, with
        // the last value, so converting them can't collide.
        facts(
            MODE,
            r#"
            let m: map[int,str] = [1: "a", 1.0: "b"]
            report m == [1: "b"] and typeof m == "map[int, str]"
            let raw = [1: "a", 1.0: "b"]
            let n: map[float,str] = raw
            report typeof raw == "map[int, str]" and n == [1: "b"] and typeof n == "map[float, str]"
            let lists = [[1]: "a", [1.0]: "b"]
            let l: map[list[float],str] = lists
            report len(l) == 1 and l[[1]] == "b" and typeof l == "map[list[float], str]"
        "#,
        );
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
            report prime_factors(12.0) == [2,2,3] and is_prime(7.0) and is_squarefree(6.0)
            report divisors(6.0) == [1,2,3,6] and primes(2.0,5.0) == [2,3,5] and primes(5.0) == [2,3,5]
            report next_prime(7.0) == 11 and prev_prime(7.0) == 5 and isqrt(16.0) == 4
            report mod_pow(2.0,3.0,5.0) == 3 and mod_inverse(3.0,7.0) == 5
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
                "prime_factors(x)",
                "is_prime(x)",
                "is_squarefree(x)",
                "divisors(x)",
                "primes(x)",
                "primes(x,10)",
                "primes(1,x)",
                "next_prime(x)",
                "prev_prime(x)",
                "isqrt(x)",
                "mod_pow(x,2,5)",
                "mod_pow(2,x,5)",
                "mod_pow(2,2,x)",
                "mod_inverse(x,5)",
                "mod_inverse(2,x)",
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
            "let n:int=2.0",
            "let n:int=-0.0",
            "let n:int=100%",
            "let n:list[int]=[1.4]",
            "let n:list[int]=[2.0]",
            "fn f(n:int) { n }; report f(2.0)",
            "fn f(n:int) { n }; report f(1.4)",
            "fn f(): int { 1.4 }; report f()",
            "let n:int=if true {1.0} else {1.4}",
        ] {
            assert!(compile_error(source).contains("int"), "{source}");
        }
        for value in [
            "1.4",
            "2.0",
            "-0.0",
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
                "fn f(x): int { x }; let n=f(x)",
                "type Box={n:int}; let b=Box {n:x}",
                "let ns:list[int]=[x]",
                "let ns:dist[int]=one_of([x])",
                "var ns:list[int]=[]; ns.push(x)",
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
        rejects(MODE, "let x ~ one_of([1.0,2.0]); let n:int=x; report n", "int");
        let out = exec_raw(
            "let x ~ one_of([1.0,2.0]); let n:int=int(x); let y ~ one_of([1,2]); let f:float=y; \
             report typeof n; report typeof x; report typeof f; report typeof y",
            &options(MODE),
        )
        .unwrap();
        assert_eq!(out.reports[0].distribution(), vec![(Value::str("int"), 1.0)]);
        assert_eq!(out.reports[1].distribution(), vec![(Value::str("float"), 1.0)]);
        assert_eq!(out.reports[2].distribution(), vec![(Value::str("float"), 1.0)]);
        assert_eq!(out.reports[3].distribution(), vec![(Value::str("int"), 1.0)]);
    }

    #[test]
    fn conversion_keeps_effect_order_and_direct_inference_paths() {
        facts(
            MODE,
            r#"
            var x = 1
            fn first(n:float, ignored) { n }
            let n = first(x, { x=2; 0 })
            let k ~ binomial(3.0,1)
            let faces ~ roll(2.0,1.0)
            report n == 1 and typeof n == "float" and x == 2
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
            let n:int=int(x)
            let exact:int=-(2^200+1)
            let exact_literal:int=-9007199254740993
            let rounded:int=int(9007199254740993.0)
            report n == 2^100 and typeof n == "int" and typeof x == "float"
            report exact == -(2^200+1) and rounded == 9007199254740992
            report exact_literal == -9007199254740993
            report bit_length(x) == 101 and (0..2^101)[x] == 2^100
            report slice(0..2^101,x,x+2.0^50)[0] == 2^100
        "#,
        );
        for source in [
            "report int(1e100)",
            "let x=1e100; let n:int=int(x); report n",
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
        for source in ["report int(1e100)", "report bit_length(1e100)"] {
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
