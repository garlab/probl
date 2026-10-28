//! A declared `dist[T]` converts a distribution's outcomes when every one it
//! can have converts. A count law's far tail, past the last 1e-18 that a
//! listing keeps, isn't an outcome, so the decision comes from its law,
//! whether it's listed or not.
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

fn facts_with(source: &str, opts: &Options) {
    let out = exec_raw(source, opts).unwrap_or_else(|e| panic!("{source}\n{e:?}"));
    assert!(!out.reports.is_empty());
    for report in out.reports {
        assert_eq!(report.distribution(), vec![(Value::Bool(true), 1.0)], "{source}");
    }
}

fn rejects_with(source: &str, opts: &Options, message: &str) {
    let e = exec_raw(source, opts).expect_err(source);
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

/// The outcome limit decides whether a count law is listed or kept as a
/// law: a declaration's decision doesn't depend on it.
fn limits(mode: Mode) -> [Options; 2] {
    [
        options(mode.clone()),
        Options {
            limits: Limits {
                max_outcomes: 8,
                ..Limits::default()
            },
            ..options(mode)
        },
    ]
}

both_modes! {
    #[test]
    fn count_laws_convert_when_a_float_holds_all_but_their_far_tail() {
        for opts in limits(MODE) {
            facts_with(
                r#"
                let a: dist[float] = poisson(3)
                report typeof a == "dist[float]"
                let g: dist[float] = geometric(0.5)
                report typeof g == "dist[float]"
                let one: dist[float] = binomial(2^53, 1)
                report typeof one == "dist[float]"
                let none: dist[float] = poisson(0)
                report typeof none == "dist[float]"
                # A rate can't be more than 10^15, far below 2^53.
                let large: dist[float] = poisson(1e15)
                report typeof large == "dist[float]"
                let p: dist[prob] = binomial(1, 30%)
                report typeof p == "dist[prob]"
                let i: dist[int] = poisson(3)
                report typeof i == "dist[int]"
            "#,
                &opts,
            );
            for (law, message) in [
                ("geometric(1e-16)", "no float holds"),
                ("geometric(4e-15)", "no float holds"),
                ("binomial(2^60 + 1, 1)", "no float holds"),
                ("binomial(2^55, 0.5)", "no float holds"),
            ] {
                // Whether or not the distribution is used.
                rejects_with(&format!("let d: dist[float] = {law}"), &opts, message);
            }
            for law in ["poisson(1e-10)", "binomial(2, 30%)", "geometric(0.5)"] {
                rejects_with(&format!("let d: dist[prob] = {law}"), &opts, "above 1");
            }
        }
    }

    #[test]
    fn a_converted_law_answers_queries_as_its_source_does_with_floats() {
        facts_with(
            r#"
            let a: dist[float] = poisson(3)
            report pmf(a, 2) == pmf(poisson(3), 2) and pmf(a, 2.0) == pmf(a, 2) and pmf(a, 2.5) == 0%
            report cdf(a, 2) == cdf(poisson(3), 2) and mean(a) == mean(poisson(3))
            report slice(support(a), 0, 3) == [0, 1, 2] and typeof support(a)[0] == "float"
            # Declaring a listed law keeps its law, so queries still know its
            # tail is all that's missing.
            let i: dist[int] = poisson(3)
            report pmf(i, 2) == pmf(poisson(3), 2)
            let lazy: dist[float] = geometric(1e-12)
            report typeof lazy == "dist[float]" and mean(lazy) == 1e12
            report typeof median(lazy) == "float" and typeof quantile(lazy, 10%) == "float"
            report pmf(lazy, 3) == pmf(geometric(1e-12), 3)
            let edge: dist[float] = geometric(5e-15)
            report pmf(edge, 2^53 + 1) == 0% and pmf(edge, 3) > 0%
        "#,
            &options(MODE),
        );
        // Listing a law too broad to list fails on the outcome limit, as
        // `float(d)` of one does.
        let e = exec_raw("let d: dist[float] = geometric(1e-12); report support(d)", &options(MODE)).unwrap_err();
        assert_eq!(e.kind, ErrorKind::Limit, "{e:?}");
        let e = exec_raw("report float(geometric(1e-12))", &options(MODE)).unwrap_err();
        assert_eq!(e.kind, ErrorKind::Limit, "{e:?}");
    }

    #[test]
    fn listed_outcomes_convert_whatever_their_weight() {
        facts_with(
            r#"
            let d: dist[float] = one_of([2^53, -2^53, 2^60])
            report typeof d == "dist[float]" and support(d) == [-2^53, 2^53, 2^60]
        "#,
            &options(MODE),
        );
        for outcomes in ["[2^53 + 1]", "[1: 99%, 2^60 + 1: 1%]", "[1: 1, 2^53 + 1: 1e-300]"] {
            rejects_with(
                &format!("let d: dist[float] = one_of({outcomes})"),
                &options(MODE),
                "no float holds",
            );
        }
    }
}

#[test]
fn a_converted_law_draws_floats() {
    let opts = options(Mode::Sample { runs: 50, seed: 3 });
    facts_with(
        "let d: dist[float] = geometric(1e-12)\nlet x ~ d\nreport typeof x == \"float\" and x >= 1",
        &opts,
    );
}

#[test]
fn unresolved_weight_with_no_law_can_t_convert() {
    // An unbounded loop leaves weight unresolved when enumerating.
    let source = "let s = simulate { var n = 0; while 50% { n += 1 }; n }\nlet d: dist[float] = s";
    rejects_with(source, &options(Mode::Enumerate), "can't be checked");
    // Outcomes that need no conversion keep their declared type.
    facts_with(
        "let s = simulate { var n = 0; while 50% { n += 1 }; n }\nlet d: dist[int] = s\nreport typeof d == \"dist[int]\"",
        &options(Mode::Enumerate),
    );
}
