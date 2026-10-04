//! Candidate comparison, collection selection, and distribution support bounds.
mod common;
use common::{compile_error, exec_raw};
use probl_engine::{ErrorKind, Limits, Options, value::Value};
use probl_sema::ir::Mode;

const MODES: [Mode; 2] = [Mode::Enumerate, Mode::Sample { runs: 50, seed: 7 }];
fn options(mode: Mode) -> Options {
    Options {
        mode: Some(mode),
        ..Options::default()
    }
}
fn facts(mode: Mode, source: &str) {
    let out = exec_raw(source, &options(mode)).unwrap_or_else(|e| panic!("{source}: {e:?}"));
    assert!(!out.reports.is_empty());
    for r in out.reports {
        assert_eq!(r.distribution(), vec![(Value::Bool(true), 1.0)], "{source}");
    }
}
fn rejects(mode: Mode, source: &str, message: &str) {
    let e = exec_raw(source, &options(mode)).expect_err(source);
    assert_eq!(e.kind, ErrorKind::Language, "{source}: {e:?}");
    assert!(e.message.contains(message), "{source}: {e:?}");
}

#[test]
fn positional_extrema_lift_without_changing_independence_or_correlation() {
    for mode in MODES {
        facts(
            mode,
            r#"
            let d=d2
            report pmf(max(d,d),1)==25% and pmf(min(d,d),2)==25%
            report maximum(max(3d6+3,2d8+1))==21
            report minimum(min(3d6+3,2d8+1))==3
            report typeof max(d6,3)=="dist[int]"
            report support(max(one_of([1.0]),1))==[1.0]
            report support(min(1,one_of([1.0])))==[1]
            let x ~ d
            report max(x,x)==x and min(x,x)==x
        "#,
        );
    }
}

#[test]
fn collection_selection_preserves_elements_types_and_first_ties() {
    for mode in MODES {
        facts(
            mode,
            r#"
            report maximum([-8,-3,-12])==-3 and minimum([-8,-3,-12])==-12
            report typeof maximum([1.0,1])=="float" and typeof minimum([1,1.0])=="int"
            report maximum([[1,9],[2,0]])==[2,0] and minimum([[1,9],[2,0]])==[1,9]
            report maximum([[1,2]])==[1,2] and minimum(["z"])=="z"
            report maximum("a🙂é")=="🙂" and minimum("a🙂é")=="a"
            report maximum(1..10^100)==10^100 and minimum(1..10^100)==1
            report maximum([date(2025,1,1),date(2026,1,1)])==date(2026,1,1)
            let rows=[{k:2,id:"a"},{k:1,id:"b"},{k:2,id:"c"},{k:1,id:"d"}]
            report rows.maximum((a,b)->a.k-b.k).id=="a"
            report rows.minimum((a,b)->a.k-b.k).id=="b"
            let z=[complex(2,3),complex(1,4),complex(0,5)]
            report z.maximum((a,b)->abs(a)-abs(b))==complex(0,5)
            report z.minimum((a,b)->abs(a)-abs(b))==complex(2,3)
            report [complex(1)].maximum((a,b)->1/0)==complex(1)
        "#,
        );
    }
}

#[test]
fn recipes_can_be_selected_but_are_never_implicitly_ordered_or_combined() {
    for mode in MODES {
        facts(
            mode.clone(),
            r#"
            let ds=[d6,d8]
            let chosen=ds.maximum((a,b)->mean(a)-mean(b))
            report maximum(chosen)==8 and pmf(chosen,1)==12.5%
            report typeof chosen=="dist[int]"
            report maximum(ds.minimum((a,b)->mean(a)-mean(b)))==6
            report pmf([d2,d2].reduce(0,max),1)==25%
        "#,
        );
        for xs in ["[d6,d8]", "[d6]", "[[d6]]", "[uniform(0,1),2]"] {
            rejects(mode.clone(), &format!("report maximum({xs})"), "compare");
        }
    }
}

#[test]
fn distribution_bounds_use_whole_support_and_preserve_exact_values() {
    for mode in MODES {
        facts(
            mode,
            r#"
            report maximum(3d8)==24 and minimum(3d8)==3
            report maximum(one_of([1:1,10^400:1e-13]))==10^400
            report minimum(one_of(["z","a"]))=="a"
            report maximum(one_of([[1,9],[2,0]]))==[2,0]
            report maximum(uniform(0,1))==1 and minimum(uniform(0,1))==0
            report minimum(exponential(2))==0
            report maximum(beta(0.5,0.5))==1
            report maximum(triangular(1,2,3))==3
            report minimum(pert(1,2,3))==1
            report maximum(one_of([uniform(0,1),10^400]))==10^400
            report minimum(one_of([uniform(0,1),-4]))==-4
        "#,
        );
    }
}

#[test]
fn invalid_populations_and_unresolved_or_unbounded_support_are_rejected() {
    for mode in MODES {
        for op in ["minimum", "maximum"] {
            for xs in ["[]", "\"\"", "1..0"] {
                rejects(mode.clone(), &format!("report {op}({xs})"), "nonempty");
            }
            for scalar in ["1", "complex(1)", "date(2026,1,1)"] {
                rejects(mode.clone(), &format!("report {op}({scalar})"), "expects");
            }
            for xs in ["[complex(1)]", "one_of([complex(1)])"] {
                rejects(mode.clone(), &format!("report {op}({xs})"), "ordering");
            }
            rejects(
                mode.clone(),
                &format!("report {op}(normal(0,1))"),
                "finite support bound",
            );
            rejects(mode.clone(), &format!("report {op}(geometric(50%))"), "unresolved");
            rejects(
                mode.clone(),
                &format!(
                    "@epsilon 0.1; let d=simulate {{ var n=0; while 50% {{n+=1}}; n }}; report {op}(one_of([d,uniform(0,1)]))"
                ),
                "unresolved",
            );
            rejects(mode.clone(), &format!("report {op}(d6,(a,b)->a-b)"), "comparator");
            rejects(mode.clone(), &format!("report {op}(uniform(0,1),(a,b)->a-b)"), "list");
        }
        rejects(mode, "report maximum(exponential(2))", "finite support bound");
    }
}

#[test]
fn top_n_always_returns_a_list_and_uses_stable_comparators() {
    for mode in MODES {
        facts(
            mode.clone(),
            r#"
            let rows=[{k:2,id:"a"},{k:1,id:"b"},{k:2,id:"c"},{k:1,id:"d"}]
            report rows.highest(2,(a,b)->a.k-b.k).map(x->x.id)==["a","c"]
            report rows.lowest(2,(a,b)->a.k-b.k).map(x->x.id)==["b","d"]
            report [3,1,2].highest(1)==[3] and [3,1,2].lowest(1)==[1]
            report [3,1,2].highest(0)==[] and [].lowest(5)==[]
            report [3,1,2].lowest(10^100)==[1,2,3]
            report "cba".highest(2)==["c","b"] and (1..3).lowest(2)==[1,2]
            report support(one_of([[1,2],[1,3]]).highest(1))==[[2],[3]]
            report support([3,1,2].highest(d2,(a,b)->a-b))==support([3,1,2].highest(d2))
        "#,
        );
        for op in ["highest", "lowest"] {
            rejects(mode.clone(), &format!("report {op}([1],-1)"), "nonnegative");
            rejects(mode.clone(), &format!("report {op}([1],1.5)"), "needs an int");
        }
    }
}

#[test]
fn comparator_validation_and_effect_restrictions_match_sort() {
    for mode in MODES {
        for op in ["minimum", "maximum", "highest", "lowest"] {
            let args = if matches!(op, "highest" | "lowest") { ",2" } else { "" };
            for xs in ["[]", "[complex(1)]", "[1,2]"] {
                rejects(
                    mode.clone(),
                    &format!("report {op}({xs}{args},0)"),
                    "comparator function",
                );
                rejects(
                    mode.clone(),
                    &format!("report {op}({xs}{args},x->0)"),
                    "takes 1 argument",
                );
            }
            for body in ["true", "prob(1)", "d1", "complex(0)"] {
                rejects(
                    mode.clone(),
                    &format!("report {op}([1,2]{args},(a,b)->{body})"),
                    "finite int or float",
                );
            }
            for body in ["{let r ~ d1; a-b}", "{observe true; a-b}", "if 50% {a-b} else {a-b}"] {
                rejects(
                    mode.clone(),
                    &format!("report {op}([1,2]{args},(a,b)->{body})"),
                    "can't branch on chances, draw values or observe",
                );
            }
        }
    }
}

#[test]
fn obsolete_arities_fail_at_compile_time_and_reduce_still_requires_seed() {
    for call in [
        "min(d6)",
        "max([1,2])",
        "highest([1,2])",
        "lowest([1,2])",
        "[1,2].reduce(max)",
    ] {
        assert!(compile_error(&format!("report {call}")).contains("takes"), "{call}");
    }
}

#[test]
fn positional_extrema_preserve_unresolved_bounds() {
    for mode in MODES {
        let out = exec_raw(
            "@epsilon 0.1; let d=simulate {var n=0; while 50% {n+=1}; n}; report max(d,100)<=200; report max(d,d)<=200",
            &options(mode),
        )
        .unwrap();
        let bounds: Vec<_> = out
            .reports
            .iter()
            .map(|r| r.groups[&Value::Unit].chance_bounds(out.unresolved))
            .collect();
        assert!(bounds[0].0 < 1.0 && bounds[0].1 == 1.0);
        assert!(bounds[1].0 < bounds[0].0);
    }
}

#[test]
fn many_positional_candidates_reduce_incrementally_and_selection_obeys_limits() {
    let source = format!("report pmf(max({}),6)", vec!["d6"; 40].join(","));
    let opts = Options {
        limits: Limits {
            max_outcomes: 64,
            max_work: 10_000,
            ..Limits::default()
        },
        ..Options::default()
    };
    let out = exec_raw(&source, &opts).unwrap();
    let p = out.reports[0].distribution()[0].0.as_f64().unwrap();
    assert!((p - (1.0 - (5.0_f64 / 6.0).powi(40))).abs() < 1e-12);
    let opts = Options {
        limits: Limits {
            max_work: 100,
            ..Limits::default()
        },
        ..Options::default()
    };
    for source in [&source, "report maximum(1..1000,(a,b)->a-b)"] {
        assert_eq!(exec_raw(source, &opts).unwrap_err().kind, ErrorKind::Limit);
    }
}
