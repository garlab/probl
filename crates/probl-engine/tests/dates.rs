//! Calendar operations and a host-provided execution snapshot.
mod common;
use common::*;
use probl_engine::value::Value;
use probl_engine::{ErrorKind, Limits, Options, dates};

#[track_caller]
fn facts(expressions: &[&str]) {
    for expression in expressions {
        assert_eq!(chance(&format!("report {expression}")), 1.0, "{expression}");
    }
}

#[test]
fn constructors_components_and_immutable_arithmetic() {
    facts(&[
        r#"date(2024, 2, 29) == date("2024-02-29")"#,
        r#"date("0001-01-01").year() == 1"#,
        r#"date("9999-12-31").year() == 9999"#,
        r#"date("2024-02-29").month() == 2 and date("2024-02-29").day() == 29"#,
        r#"date("2024-02-29") + days(1) == date("2024-03-01")"#,
        r#"date("2024-03-01") - date("2024-02-28") == 2"#,
        r#"date("1969-12-31") + weeks(1) == date("1970-01-07")"#,
        r#"date("2024-02-15").start_of_month() == date("2024-02-01")"#,
        r#"date("2024-02-15").end_of_month() == date("2024-02-29")"#,
        r#"date("1900-02-01").end_of_month() == date("1900-02-28")"#,
        r#"date("2024-01-31").add_months(1) == date("2024-02-29")"#,
        r#"date("2024-01-31").add_months(2) == date("2024-03-31")"#,
        r#"date("2024-03-31").add_months(-1) == date("2024-02-29")"#,
        r#"date("2024-02-29").add_years(1) == date("2025-02-28")"#,
        r#"date("2024-02-29").add_years(4) == date("2028-02-29")"#,
        r#"date("9999-12-01").end_of_month() == date("9999-12-31")"#,
        r#"[date(2024, 2, 29): 7][date("2024-02-29")] == 7"#,
        r#"{ var d = date("2024-01-31"); let next = d.add_months(1); d == date("2024-01-31") and next == date("2024-02-29") }"#,
    ]);
}

#[test]
fn working_calendars_are_explicit_and_exclude_the_start() {
    facts(&[
        r#"date("2026-09-28").weekday() == "Monday""#,
        r#"date("2026-09-28").is_workday()"#,
        r#"not date("2026-10-03").is_workday()"#,
        r#"not date("2026-10-02").is_workday([date("2026-10-02")])"#,
        r#"date("2026-10-01").add_workdays(1, [date("2026-10-02")]) == date("2026-10-05")"#,
        r#"date("2026-10-05").add_workdays(-1, [date("2026-10-02")]) == date("2026-10-01")"#,
        r#"date("2026-10-03").add_workdays(1) == date("2026-10-05")"#,
        r#"date("2026-10-04").add_workdays(-1) == date("2026-10-02")"#,
        r#"date("2026-10-03").add_workdays(0) == date("2026-10-03")"#,
        r#"date("2026-10-02").add_workdays(0, [date("2026-10-02")]) == date("2026-10-02")"#,
        r#"date("2026-10-01").add_workdays(1, [date("2026-10-05"), date("2026-10-02"), date("2026-10-02"), date("2026-10-03")]) == date("2026-10-06")"#,
        r#"date("2026-10-02").add_workdays(1, [date("2026-10-02")]) == date("2026-10-05")"#,
    ]);
    let options = Options {
        limits: Limits {
            max_work: 100,
            ..Limits::default()
        },
        ..Options::default()
    };
    assert!(exec_raw(r#"report date("2000-01-03").add_workdays(1_000_000)"#, &options).is_ok());
}

#[test]
fn holiday_calendars_and_weekday_text_obey_resource_limits() {
    use probl_engine::{builtins::call_plain, dist::Budget};
    use probl_sema::builtins::Builtin as B;

    let d = Value::Date(dates::parse("2026-10-01").unwrap());
    let holidays = Value::list(vec![d.clone(); 4096]);
    for (builtin, args) in [
        (B::AddWorkdays, vec![d.clone(), Value::Int(1.into()), holidays.clone()]),
        (B::IsWorkday, vec![d.clone(), holidays]),
    ] {
        for mut budget in [
            Budget {
                work_left: 4096,
                ..Budget::unlimited()
            },
            Budget {
                max_collection: 4095,
                ..Budget::unlimited()
            },
        ] {
            assert_eq!(
                call_plain(builtin, &args, &mut budget).unwrap_err().kind,
                ErrorKind::Limit
            );
        }
    }
    let mut budget = Budget {
        max_string_bytes: 1,
        ..Budget::unlimited()
    };
    assert_eq!(
        call_plain(B::Weekday, &[d], &mut budget).unwrap_err().kind,
        ErrorKind::Limit
    );
}

#[test]
fn invalid_dates_offsets_and_calendars_are_language_errors() {
    assert!(compile_error("report date(2026, 1)").contains("one ISO string or three integers"));
    for expression in [
        r#"date("0000-01-01")"#,
        r#"date("10000-01-01")"#,
        r#"date("9223372036854775807-01-01")"#,
        r#"date("2026-1-01")"#,
        r#"date("2026-02-29")"#,
        r#"date("2026-01-01T12:00:00Z")"#,
        "date(2026, 13, 1)",
        "date(2026, 2, 30)",
        "date(2026.4, 1, 1)",
        "date(2026, true, 1)",
        "date(10^100, 1, 1)",
        r#"date("0001-01-01") - 1"#,
        r#"date("9999-12-31") + 1"#,
        r#"date("9999-12-31").add_months(1)"#,
        r#"date("0001-01-01").add_years(-1)"#,
        r#"date("2026-01-01").add_years(10^100)"#,
        r#"date("2026-01-01").add_workdays(-9223372036854775808)"#,
        r#"date("2026-01-01").add_months(1.4)"#,
        r#"date("2026-01-01").add_workdays(1.4)"#,
        r#"date("2026-01-01").add_workdays(0, [1])"#,
        r#"date("2026-01-01").is_workday(["2026-01-01"])"#,
        r#"date("2026-01-01").is_workday("2026-01-01")"#,
        "year(2026)",
        "month(1)",
        "day(1)",
        "start_of_month(1)",
        "end_of_month(1)",
    ] {
        let e = exec_raw(&format!("report {expression}"), &Options::default()).unwrap_err();
        assert_eq!(e.kind, ErrorKind::Language, "{expression}: {e:?}");
    }
}

#[test]
fn calendar_functions_lift_including_components_and_holidays() {
    facts(&[
        r#"date(2024, 2, one_of([28, 29])).month() == 2"#,
        r#"one_of([date("2026-01-01"), date("2026-01-31")]).end_of_month() == date("2026-01-31")"#,
        r#"one_of([date("2026-01-01"), date("2026-01-31")]).start_of_month() == date("2026-01-01")"#,
        r#"date("2026-09-28").add_workdays(d6).is_workday()"#,
        r#"date("2024-02-29").add_years(one_of([1, 2, 3])).day() == 28"#,
        r#"date("2026-10-02").add_workdays(1, one_of([[], [date("2026-10-03")]])) == date("2026-10-05")"#,
    ]);
    let outcomes = distribution(r#"report date("2026-01-31").add_months(one_of([1, 2]))"#);
    assert_eq!(
        outcomes,
        vec![
            (Value::Date(dates::parse("2026-02-28").unwrap()), 0.5),
            (Value::Date(dates::parse("2026-03-31").unwrap()), 0.5)
        ]
    );
}

#[test]
fn today_is_one_snapshot_across_worlds_samples_calls_and_simulate() {
    let source = r#"
        fn anchor() -> date { today }
        let captured = today
        let n ~ d6
        let nested = simulate { let m ~ d6; today + days(m - m) }
        report captured == anchor() and nested == today and today + n - n == captured
        report today
    "#;
    let program = probl_sema::compile(source).0.unwrap();
    for stamp in ["2024-02-29", "2026-09-29"] {
        let snapshot = dates::parse(stamp).unwrap();
        for threads in [1, 4] {
            for mode in [None, Some(probl_sema::ir::Mode::Sample { runs: 2500, seed: 7 })] {
                let options = Options {
                    today: Some(snapshot),
                    mode,
                    limits: Limits {
                        max_threads: threads,
                        ..Limits::default()
                    },
                    ..Options::default()
                };
                let out = probl_engine::run(&program, &options, &mut |_| {}).unwrap();
                assert_eq!(out.today, Some(snapshot));
                assert_eq!(out.reports[0].chance(), Some(1.0));
                assert_eq!(out.reports[1].distribution(), vec![(Value::Date(snapshot), 1.0)]);
            }
        }
    }
}

#[test]
fn today_is_a_shadowable_constant_and_requires_host_context() {
    facts(&[
        r#"{ let today = date("2020-01-01"); today.year() == 2020 }"#,
        r#"{ var today = 1; today += 1; today == 2 }"#,
    ]);
    assert_eq!(chance("enum S { today, other }\nreport today == S.today"), 1.0);
    assert_eq!(chance("fn today() { 7 }\nreport today() == 7"), 1.0);
    assert!(compile_error("report today()").contains("constant, not a function"));
    assert!(compile_error("today = date(2026, 1, 1)").contains("constant"));
    assert!(error("report today").contains("host did not supply"));
    let options = Options {
        today: Some(i32::MAX),
        ..Options::default()
    };
    assert_eq!(
        exec_raw("report today", &options).unwrap_err().kind,
        ErrorKind::Language
    );
}
