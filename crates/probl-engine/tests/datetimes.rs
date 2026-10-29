//! Canonical temporal literals and timezone-free, UTC-normalized datetimes.
mod common;
use common::*;
use probl_engine::{Options, dates::DateTime, value::Value};
use probl_sema::ir::Mode;

fn facts(source: &str) {
    for mode in [Mode::Enumerate, Mode::Sample { runs: 40, seed: 7 }] {
        let out = exec_raw(
            source,
            &Options {
                mode: Some(mode),
                ..Options::default()
            },
        )
        .unwrap();
        assert!(!out.reports.is_empty());
        for report in out.reports {
            assert_eq!(report.distribution(), vec![(Value::Bool(true), 1.0)], "{source}");
        }
    }
}

#[test]
fn literals_are_typed_constants_everywhere() {
    facts(
        r#"
        let d: date = '2024-02-29'
        let t: datetime = '2024-02-29T12:34:56.123456789'
        report typeof d == "date" and typeof t == "datetime"
        report d == date(2024,2,29)
        report t == datetime(2024,2,29,12,34,56,123456789)
        report datetime(2024,2,29,0,0,0) == '2024-02-29T00:00:00'
        report t.year() == 2024 and t.month() == 2 and t.day() == 29
        report t.hour() == 12 and t.minute() == 34 and t.second() == 56 and t.nanosecond() == 123456789
        fn echo(x: date or datetime): date or datetime { x }
        let f = echo
        report f(d) == d and f(t) == t
        let xs: list[date or datetime] = [d,t]
        report typeof xs == "list[date or datetime]"
        report match d { '2024-02-29' => true, _ => false }
        report match t { '2024-02-29T12:34:56.123456789Z' => true, _ => false }
        report "date: {'2024-02-29'}" == "date: 2024-02-29"
        report parse_date(env.get("UNSET_TEMPORAL_TEST", "2024-02-29")) == d
        report parse_datetime("2024-02-29T12:34:56.123456789") == t
        report str('2024-02-29T00:00:00.120000000') == "2024-02-29T00:00:00.12"
    "#,
    );
}

#[test]
fn offsets_normalize_before_comparison_and_identity() {
    facts(
        r#"
        let t = '2027-01-31T09:30:00+02:00'
        report t == '2027-01-31T07:30:00' and str(t) == "2027-01-31T07:30:00"
        report '2027-01-01T00:15:00+01:00' == '2026-12-31T23:15:00'
        report '2024-02-29T23:45:00-01:30' == '2024-03-01T01:15:00Z'
        report '1970-01-01T00:00:00Z' == '1970-01-01T00:00:00+00:00'
        report '1970-01-01T00:00:00-00:00' == '1970-01-01T00:00:00'
        report parse_datetime("2027-01-31T09:30:00+02:00") == t
        let xs = set(t, '2027-01-31T07:30:00Z')
        report len(xs) == 1
        let m = [t: 1, '2027-01-31T07:30:00Z': 2]
        report len(m) == 1 and m[t] == 2
        report pmf(one_of([t, '2027-01-31T07:30:00Z']), t) == 100%
        report '2027-01-31T07:30:00.000000001' > t
        report '2027-01-31' != '2027-01-31T00:00:00'
    "#,
    );
}

#[test]
fn calendar_and_clock_operations_are_exact_and_lift() {
    facts(
        r#"
        let t = '2024-01-31T23:59:59.999999999'
        report t.add_nanoseconds(1) == '2024-02-01T00:00:00'
        report '1970-01-01T00:00:00'.add_nanoseconds(-1) == '1969-12-31T23:59:59.999999999'
        report t.add_months(1) == '2024-02-29T23:59:59.999999999'
        report t.add_years(1) == '2025-01-31T23:59:59.999999999'
        report t.add_days(1) == '2024-02-01T23:59:59.999999999'
        report '2024-02-29'.add_days(1) == '2024-03-01'
        report t.add_hours(1).hour() == 0 and t.add_minutes(1).minute() == 0
        report t.add_seconds(1).nanosecond() == 999999999
        report t.start_of_month() == '2024-01-01T23:59:59.999999999'
        report t.end_of_month() == t
        report '2026-10-02T15:00:00'.add_workdays(1) == '2026-10-05T15:00:00'
        report '2026-10-02T15:00:00'.is_workday() and '2026-10-02T15:00:00'.weekday() == "Friday"
        report t.add_days(one_of([1,2])).month() == 2
        report datetime(2024,2,one_of([28,29]),0,0,0).month() == 2
        report parse_datetime(one_of(["2024-02-28T00:00:00", "2024-02-29T00:00:00"])).month() == 2
        let long = '0001-01-01T00:00:00'.add_nanoseconds(10000000000000000000)
        report long.add_nanoseconds(-10000000000000000000) == '0001-01-01T00:00:00'
        report str('0001-01-01T00:00:00') == "0001-01-01T00:00:00"
        report str('9999-12-31T23:59:59.999999999') == "9999-12-31T23:59:59.999999999"
    "#,
    );
}

#[test]
fn ordered_queries_preserve_datetime_values() {
    facts(
        r#"
        let a = '2024-01-01T00:00:00'
        let b = a.add_nanoseconds(1)
        report minimum([b,a]) == a and maximum([a,b]) == b
        report quantile([b,a],50%) == a and median_low([b,a]) == a and median_high([b,a]) == b
        report cdf(one_of([a,b]),a) == 50%
        report [b,a].sort() == [a,b]
        report median_low(one_of([b,a])) == a
        report median([a,b]) == a
        report median([a,a.add_nanoseconds(2)]) == b
        report median(['1969-12-31T23:59:59.999999999','1970-01-01T00:00:00']) == '1969-12-31T23:59:59.999999999'
    "#,
    );
}

#[test]
fn invalid_literals_fail_during_compilation_and_parsing_fails_at_runtime() {
    for text in [
        "hello",
        "",
        "2026-02-29",
        "0000-01-01",
        "2026-1-01",
        "2026-01-01T24:00:00",
        "2026-01-01T00:00:60",
        "2026-01-01T00:00",
        "2026-01-01T00:00:00.",
        "2026-01-01T00:00:00.1234567890",
        "2026-01-01T00:00:00+24:00",
        "2026-01-01T00:00:00+01:60",
        "2026-01-01T00:00:00 Europe/London",
        "0001-01-01T00:00:00+00:01",
        "9999-12-31T23:59:59-00:01",
        "🙂",
    ] {
        assert!(
            compile_error(&format!("if false {{ report '{text}' }}")).contains("valid date, datetime or duration"),
            "{text}"
        );
        assert!(
            error(&format!("let text=\"{text}\"; report parse_datetime(text)")).contains("datetime"),
            "{text}"
        );
    }
    assert!(compile_error("report '2027-01-01").contains("unterminated"));
    assert!(compile_error("report 'hello'").contains("double quotes"));
    assert!(compile_error("report date(\"2027-01-01\")").contains("parse_date"));
    assert!(error("let f=date; report f(\"2027-01-01\")").contains("parse_date"));
    assert!(!compile_error("let x: str = '2027-01-01'").is_empty());
    for source in [
        "report datetime(2026,1,1,24,0,0)",
        "report datetime(2026,1,1,0,0,0,1000000000)",
        "report datetime(2026,1,1,0.0,0,0)",
        "report '0001-01-01T00:00:00'.add_nanoseconds(-1)",
        "report '9999-12-31T23:59:59.999999999'.add_nanoseconds(1)",
        "report '2026-01-01T00:00:00'.add_seconds(1.0)",
        "report '2026-01-01T00:00:00'.add_nanoseconds(10^100)",
    ] {
        assert!(!error(source).is_empty(), "{source}");
    }
}

#[test]
fn nested_display_round_trips_as_source() {
    let source = "['2024-02-29', ['2024-02-29T12:00:00.000000001'], ['2024-01-01': '2024-01-01T01:30:00+01:30']]";
    let value = distribution(&format!("report {source}"))[0].0.clone();
    let text = value.to_string();
    assert!(text.contains("'2024-02-29'"));
    assert!(text.contains("'2024-01-01T00:00:00'"));
    assert_eq!(distribution(&format!("report {text}"))[0].0, value);
    assert_eq!(
        DateTime::parse("2024-01-01T00:00:00.1Z").unwrap().format(),
        "2024-01-01T00:00:00.1"
    );
}
