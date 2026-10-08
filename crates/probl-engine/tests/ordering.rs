//! Public ordering, custom comparators, and finite order statistics.
mod common;
use common::*;
use probl_engine::{ErrorKind, FailureMode, Limits, Options, value::Value};
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
            const MODE: Mode = Mode::Sample { runs: 50, seed: 7 };
            $($test)*
        }
    };
}

both_modes! {
    #[test]
    fn complex_values_can_be_sorted_by_a_chosen_real_measure() {
        facts(
            MODE,
            r#"
            let xs = [complex(2,3), complex(1,4), complex(0,5)]
            let by_real = (a,b) -> real(a)-real(b)
            report xs.sort(by_real) == [complex(0,5), complex(1,4), complex(2,3)]
            report sort_desc(xs,by_real) == xs
            report reverse(xs).sort((a,b) -> abs(a)-abs(b)) == xs
            report xs == [complex(2,3), complex(1,4), complex(0,5)]
            report [complex(2,3)].sort(by_real) == [complex(2,3)]
            report [].sort(by_real) == []
        "#,
        );
    }

    #[test]
    fn sorting_is_stable_in_both_directions_and_preserves_numeric_types() {
        facts(
            MODE,
            r#"
            let rows = [{k:2,id:"a"},{k:1,id:"b"},{k:2,id:"c"},{k:1,id:"d"}]
            report rows.sort((a,b)->a.k-b.k).map(x->x.id) == ["b","d","a","c"]
            report rows.sort_desc((a,b)->a.k-b.k).map(x->x.id) == ["a","c","b","d"]
            report rows.sort((a,b)->0) == rows and rows.sort_desc((a,b)->-0.0) == rows
            let xs = [1.0,1,prob(1),2]
            report xs.sort().map(x->typeof x) == ["float","int","prob","int"]
            report xs.sort_desc().map(x->typeof x) == ["int","float","int","prob"]
            report xs.sort((a,b)->a-b).map(x->typeof x) == ["float","int","prob","int"]
        "#,
        );
    }

    #[test]
    fn custom_ordering_supports_sequences_captures_and_large_integer_results() {
        facts(
            MODE,
            r#"
            let direction = -1
            report (1..4).sort((a,b)->direction*(a-b)) == [4,3,2,1]
            report "a🙂é".sort((a,b)->if a==b {0} else if a>b {-1} else {1}) == ["🙂","é","a"]
            report [true,false,true].sort((a,b)->if a==b {0} else if a {1} else {-1}) == [false,true,true]
            report [3,1,2].sort((a,b)->(a-b)*10^400) == [1,2,3]
            report [3,1,2].sort((a,b)->(a-b)*1e-300) == [1,2,3]
            report [date(2026,1,2),date(2025,1,3)].sort((a,b)->a-b) == [date(2025,1,3),date(2026,1,2)]
            report ["bbb","a","cc"].sort((a,b)->len(a)-len(b)) == ["a","cc","bbb"]
        "#,
        );
    }

    #[test]
    fn comparator_type_and_arity_are_checked_even_without_comparisons() {
        for xs in ["[]", "[complex(1)]", "[2,1]"] {
            rejects(MODE, &format!("report sort({xs},0)"), "comparator function");
            rejects(MODE, &format!("report sort_desc({xs},x->0)"), "takes 1 argument");
            rejects(MODE, &format!("report sort({xs},(a,b,c)->0)"), "takes 3 argument");
        }
        facts(MODE, "report [].sort((a,b)->1/0)==[] and [1].sort((a,b)->1/0)==[1]");
        assert!(compile_error("report sort([1],(a,b)->0,1)").contains("takes"));
    }

    #[test]
    fn comparators_require_finite_signed_numbers_and_propagate_errors() {
        for result in [
            "true",
            "false",
            "prob(1)",
            "complex(0)",
            "\"less\"",
            "[]",
            "d1",
            "simulate { 0 }",
        ] {
            for name in ["sort", "sort_desc"] {
                rejects(
                    MODE,
                    &format!("report {name}([2,1],(a,b)->{result})"),
                    "finite int or float",
                );
            }
        }
        rejects(MODE, "report [2,1].sort((a,b)->1/0)", "division by zero");
        rejects(MODE, "report [2,1].sort((a,b)->mean(lognormal(1000,1)))", "finite");
    }

    #[test]
    fn comparator_callbacks_share_collection_effect_restrictions() {
        for body in [
            "{ let r ~ d1; a-b }",
            "{ let r = ~d1; a-b }",
            "{ observe true; a-b }",
            "{ score 100%; a-b }",
            "if 50% {a-b} else {a-b}",
            "{ var deck=bag([1]); let r=deck.take(); a-b }",
            "{ let xs=[1].map(x->{observe true; x}); a-b }",
        ] {
            for name in ["sort", "sort_desc"] {
                rejects(
                    MODE,
                    &format!("report {name}([2,1],(a,b)->{body})"),
                    "can't branch on chances",
                );
            }
        }
        rejects(
            MODE,
            "fn cmp(a,b) { let r ~ d1; a-b }; let cached=cmp(2,1); report [2,1].sort((a,b)->cmp(a,b))",
            "can't branch on chances",
        );
        facts(
            MODE,
            "report [2,1].sort((a,b)->mean(simulate {let r ~ d6; a-b+r-r})) == [1,2]",
        );
    }

    #[test]
    fn custom_sort_lifts_over_populations_without_drawing() {
        facts(
            MODE,
            r#"
            let recipes = one_of([[3,1,2],[5,4]])
            let ordered = recipes.sort((a,b)->a-b)
            report typeof ordered == "dist[list[int]]"
            report pmf(ordered,[1,2,3]) == 50% and pmf(ordered,[4,5]) == 50%
            report pmf(recipes,[3,1,2]) == 50%
        "#,
        );
    }

    #[test]
    fn default_sort_rejects_unordered_singletons_and_hidden_nested_values() {
        for xs in [
            "[complex(1)]",
            "[{x:1}]",
            "[d1]",
            "[true]",
            "[[0,complex(1)],[1,0]]",
            "[[0,{x:1}],[1,0]]",
        ] {
            for name in ["sort", "sort_desc"] {
                let e = exec_raw(&format!("report {name}({xs})"), &options(MODE)).expect_err(xs);
                assert_eq!(e.kind, ErrorKind::Language, "{xs}: {e:?}");
            }
        }
        facts(MODE, "report sort([])==[] and sort([[]])==[[]]");
    }

    #[test]
    fn medians_quantiles_sort_and_cdf_agree_on_nested_numeric_ordering() {
        for first in ["1", "1.0", "prob(1)"] {
            for second in ["1", "1.0", "prob(1)"] {
                for reverse in [false, true] {
                    let items = if reverse {
                        format!("[[{second},100],[{first},0]]")
                    } else {
                        format!("[[{first},0],[{second},100]]")
                    };
                    for input in ["xs", "one_of(xs)"] {
                        facts(
                            MODE,
                            &format!(
                                r#"
                            let xs={items}
                            let d={input}
                            report median_low(d)==sort(xs)[0] and median_high(d)==sort(xs)[1]
                            report quantile(d,50%)==sort(xs)[0] and quantile(d,100%)==sort(xs)[1]
                            report cdf(d,quantile(d,50%))==50%
                        "#
                            ),
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn finite_quantile_endpoints_and_close_boundaries_preserve_tails() {
        for input in ["[1,4]", "one_of([1,4])"] {
            facts(
                MODE,
                &format!(
                    r#"
                let d={input}
                report quantile(d,0%)==1 and quantile(d,100%)==4
                report quantile(d,0.49999999999999994)==1
                report quantile(d,0.5)==1
                report quantile(d,0.5000000000000001)==4
                report median(d)==2.5
            "#
                ),
            );
        }
        for tail in ["1e-13", "1e-16", "1e-30", "1e-300"] {
            facts(
                MODE,
                &format!(
                    r#"
                let upper=one_of([0:1,1000000:{tail}])
                let lower=one_of([-1000000:{tail},0:1])
                report pmf(upper,1000000)>0% and quantile(upper,100%)==1000000
                report pmf(lower,-1000000)>0% and quantile(lower,0%) == -1000000
            "#
                ),
            );
        }
        facts(
            MODE,
            r#"
            let d=one_of([0:1,1:1e-13,2:1])
            report quantile(d,0.5)==1
            report quantile(d,0.4999999999999)==0
            report quantile(d,0.5000000000001)==2
        "#,
        );
    }

    #[test]
    fn quantiles_validate_lists_and_distributions_identically() {
        for xs in [
            "[1,\"a\"]",
            "[{x:1}]",
            "[complex(1)]",
            "[[0,complex(1)],[1,0]]",
            "[[0,\"a\"],[0,1]]",
        ] {
            for population in [xs.to_string(), format!("one_of({xs})")] {
                for q in ["0%", "50%", "100%"] {
                    let src = format!("report quantile({population},{q})");
                    let e = exec_raw(&src, &options(MODE)).expect_err(&src);
                    assert_eq!(e.kind, ErrorKind::Language, "{src}: {e:?}");
                }
            }
        }
        facts(
            MODE,
            r#"
            report quantile([false,true],50%)==false and quantile(bernoulli(50%),100%)==true
            enum Size { Small, Large }
            report quantile(one_of([Large,Small]),0%)==Small
        "#,
        );
    }
}

#[test]
fn comparator_failure_stops_calls_immediately() {
    let src = "report [4,3,2,1].sort((a,b)->{print(\"compare\"); 1/0})";
    for mode in [Mode::Enumerate, Mode::Sample { runs: 1, seed: 7 }] {
        let (program, diagnostics) = probl_sema::compile(src);
        assert!(program.is_some(), "{diagnostics:?}");
        let mut prints = Vec::new();
        let total = Options {
            on_error: Some(FailureMode::Total),
            ..options(mode)
        };
        let e = probl_engine::run(&program.unwrap(), &total, &mut |s| prints.push(s.to_owned())).unwrap_err();
        assert_eq!(e.kind, ErrorKind::Language);
        assert_eq!(prints, vec!["compare"]);
    }
}

#[test]
fn comparator_prints_are_visible_to_memoization_and_draw_scheduling() {
    for name in ["sort", "sort_desc"] {
        for source in [
            format!(
                "fn sorted() {{ [2,1].{name}((a,b)->{{print(\"compare\"); a-b}}) }}; let a=sorted(); let b=sorted(); report a==b"
            ),
            format!("let r ~ d2; let xs=[2,1].{name}((a,b)->{{print(\"compare\"); a-b}}); report r"),
        ] {
            let (program, diagnostics) = probl_sema::compile(&source);
            assert!(program.is_some(), "{diagnostics:?}");
            let mut prints = Vec::new();
            probl_engine::run(&program.unwrap(), &Options::default(), &mut |s| {
                prints.push(s.to_owned())
            })
            .unwrap();
            assert_eq!(prints.len(), 2, "{source}: {prints:?}");
        }
    }
}

#[test]
fn inconsistent_comparators_cannot_panic_or_lose_items() {
    for comparator in ["(a,b)->1", "(a,b)->-1", "(a,b)->(a mod 3)-(b mod 2)"] {
        let src = format!("report (1..513).sort({comparator}).sort() == (1..513).map(x->x)");
        facts(Mode::Enumerate, &src);
    }
}

#[test]
fn sorting_and_comparator_work_respect_limits() {
    for source in [
        "report sort(1..1000)",
        "report (1..1000).sort((a,b)->a-b)",
        "report [2,1].sort((a,b)->{var n=0; repeat 10000 {n+=1}; a-b})",
        "report quantile(one_of(1..1000),50%)",
    ] {
        let opts = Options {
            limits: Limits {
                max_work: 5000,
                ..Limits::default()
            },
            ..Options::default()
        };
        let e = exec_raw(source, &opts).expect_err(source);
        assert_eq!(e.kind, ErrorKind::Limit, "{source}: {e:?}");
    }
}
