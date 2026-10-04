//! Sequence and positional extrema share finite-recipe lifting semantics.
mod common;

use common::exec_raw;
use probl_engine::{ErrorKind, Limits, Options, value::Value};
use probl_sema::ir::Mode;

const MODES: [Mode; 2] = [Mode::Enumerate, Mode::Sample { runs: 50, seed: 7 }];

fn options(mode: &Mode) -> Options {
    Options {
        mode: Some(mode.clone()),
        ..Options::default()
    }
}

fn facts(mode: Mode, source: &str) {
    let out = exec_raw(source, &options(&mode)).unwrap_or_else(|e| panic!("{source}: {e:?}"));
    for r in out.reports {
        assert_eq!(r.distribution(), vec![(Value::Bool(true), 1.0)], "{source}");
    }
}

#[test]
fn list_candidates_match_positional_candidates() {
    for mode in MODES {
        for op in ["min", "max"] {
            for args in ["d6,3", "3d6+3,2d8+1", "d4,2,d6", "d2,d2,d2", "1,23,4"] {
                let source = format!("report {op}([{args}]); report {op}({args})");
                let out = exec_raw(&source, &options(&mode)).unwrap();
                let a = out.reports[0].distribution();
                let b = out.reports[1].distribution();
                assert_eq!(a.len(), b.len(), "{source}");
                for ((x, p), (y, q)) in a.iter().zip(&b) {
                    assert_eq!(x, y, "{source}");
                    assert!((p / q - 1.0).abs() < 1e-12, "{source}: {p} versus {q}");
                }
            }
        }
    }
}

#[test]
fn singleton_recipes_and_ties_preserve_types() {
    for mode in MODES {
        facts(
            mode,
            r#"
            report typeof min([d1])=="dist[int]" and typeof max([1])=="int"
            report support(min([d6]))==[1,2,3,4,5,6] and pmf(max([d6]),6)==prob(1/6)
            report support(max([one_of([1.0]),1]))==[1.0]
            report support(min([1,one_of([1.0])]))==[1]
            report min(["z"])=="z" and max([[1,2]])==[1,2]
        "#,
        );
    }
}

#[test]
fn recipes_are_independent_and_drawn_values_stay_correlated() {
    for mode in MODES {
        facts(
            mode,
            r#"
            let d=d2
            let xs=[d,d]
            fn largest(xs) { max(xs) }
            report pmf(largest(xs),1)==25% and pmf(min(xs),2)==25%
            report typeof xs=="list[dist[int]]"
            let x ~ d
            report max([x,x])==x and min([x,x])==x
        "#,
        );
    }
}

#[test]
fn nested_lists_remain_values_and_distributions_of_lists_still_lift() {
    for mode in MODES {
        facts(
            mode,
            r#"
            report max([[1,9],[2,0]])==[2,0] and min([[1,9],[2,0]])==[1,9]
            let lists=one_of([[d2,0],[1,d2]])
            report pmf(max(lists),1)==50% and pmf(max(lists),2)==50%
            report support(max([one_of([[1,9],[2,0]]),[1,10]]))==[[1,10],[2,0]]
        "#,
        );
    }
}

#[test]
fn unordered_candidates_and_empty_lists_still_fail() {
    for mode in MODES {
        for op in ["min", "max"] {
            for arg in [
                "[]",
                "d6",
                "[complex(1)]",
                "[one_of([complex(1)])]",
                "[one_of([1,\"x\"]),2]",
                "[uniform(0,1),2]",
                "[[d6]]",
                "[one_of([{x:1}])]",
            ] {
                let source = format!("report {op}({arg})");
                let e = exec_raw(&source, &options(&mode)).expect_err(&source);
                assert_eq!(e.kind, ErrorKind::Language, "{source}: {e:?}");
            }
        }
    }
}

#[test]
fn list_extrema_preserve_unresolved_bounds() {
    for mode in MODES {
        for op in ["min", "max"] {
            let source = format!(
                r#"
                @epsilon 0.1
                let d=simulate {{var n=0; while 50% {{n+=1}}; n}}
                report {op}([d,100])<=200
                report {op}(d,100)<=200
                report {op}([d,d])<=200
                report {op}(d,d)<=200
            "#
            );
            let out = exec_raw(&source, &options(&mode)).unwrap();
            let bounds: Vec<_> = out
                .reports
                .iter()
                .map(|r| r.groups[&Value::Unit].chance_bounds(out.unresolved))
                .collect();
            assert_eq!(bounds[0], bounds[1]);
            assert_eq!(bounds[2], bounds[3]);
            assert!(bounds[0].0 < 1.0 && bounds[0].1 == 1.0);
            assert!(bounds[2].0 < bounds[0].0);
        }
    }
}

#[test]
fn many_candidates_reduce_incrementally_and_obey_work_limits() {
    let source = "let xs=(1..40).map(x->d6); report pmf(max(xs),6)";
    let opts = Options {
        limits: Limits {
            max_outcomes: 64,
            max_work: 10_000,
            ..Limits::default()
        },
        ..Options::default()
    };
    let out = exec_raw(source, &opts).unwrap();
    let p = out.reports[0].distribution()[0].0.as_f64().unwrap();
    assert!((p - (1.0 - (5.0_f64 / 6.0).powi(40))).abs() < 1e-12);
    let opts = Options {
        limits: Limits {
            max_work: 100,
            ..Limits::default()
        },
        ..Options::default()
    };
    assert_eq!(exec_raw(source, &opts).unwrap_err().kind, ErrorKind::Limit);
}
