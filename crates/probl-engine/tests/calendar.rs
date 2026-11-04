mod common;
use common::*;
use probl_engine::{ErrorKind, Limits, Options, dates, value::Value};
use probl_sema::ir::Mode;

fn facts(source: &str) {
    for mode in [Mode::Enumerate, Mode::Sample { runs: 200, seed: 7 }] {
        let out = exec_raw(
            source,
            &Options {
                mode: Some(mode),
                ..Options::default()
            },
        )
        .unwrap();
        for r in out.reports {
            assert_eq!(r.chance(), Some(1.0), "{source}");
        }
    }
}

#[test]
fn calendar_queries_cover_leap_days_and_iso_year_boundaries() {
    facts(
        r#"
        report day_of_year('2024-03-01') == 61 and day_of_year('2023-03-01') == 60
        report day_of_year('2024-12-31') == 366 and day_of_year('2023-12-31') == 365
        report iso_week('2021-01-01') == 53 and iso_week_year('2021-01-01') == 2020
        report iso_week('2018-12-31') == 1 and iso_week_year('2018-12-31') == 2019
        report iso_week('2021-01-04') == 1 and iso_week_year('2021-01-04') == 2021
        report iso_week('0001-01-01') == 1 and iso_week_year('0001-01-01') == 1
        report iso_week('9999-12-31') == 52 and iso_week_year('9999-12-31') == 9999
        report [1,4,7,10].map(m -> quarter(date(2024,m,1))) == [1,2,3,4]
        report quarter('2024-03-31') == 1 and quarter('2024-12-31') == 4
        let t = '2024-01-01T00:15:00+01:00'
        report quarter(t) == 4 and day_of_year(t) == 365 and iso_week_year(t) == 2023
        report day_of_year(one_of(['2024-01-01','2024-01-02'])) <= 2
    "#,
    );
}

#[test]
fn explicit_date_datetime_conversions_lift_and_keep_normalized_fields() {
    facts(
        r#"
        let d = '2024-02-29'
        let t = '2024-03-01T00:15:00.123456789+01:00'
        report datetime(d) == '2024-02-29T00:00:00'
        report date(t) == d and datetime(t) == t and date(d) == d
        report datetime(date(t)) == '2024-02-29T00:00:00'
        report d.datetime().date() == d and t.date() == d
        report datetime('0001-01-01') == '0001-01-01T00:00:00'
        report date('9999-12-31T23:59:59.999999999') == '9999-12-31'
        let convert = datetime
        report convert(d) == datetime(d)
        report typeof datetime(one_of([d,d+1])) == "dist[datetime]"
        report pmf(date(one_of([t,t+'1h'])), d) == 50%
    "#,
    );
    for source in [
        "report date(1)",
        "report datetime(1)",
        "report date(\"2024-01-01\")",
        "report datetime(\"2024-01-01T00:00:00\")",
        "let f=datetime; report f(1,2)",
    ] {
        assert!(!error(source).is_empty(), "{source}");
    }
    for source in [
        "report date(2024,1)",
        "report datetime(2024,1,1)",
        "report datetime(2024,1,1,0,0)",
    ] {
        assert!(!compile_error(source).is_empty(), "{source}");
    }
}

#[test]
fn date_distributions_are_discrete_and_estimates_preserve_translation() {
    facts(
        r#"
        let d = uniform('2024-02-28','2024-03-01')
        report typeof d == "dist[date]" and len(support(d)) == 3
        report abs(pmf(d,'2024-02-29') - 1/3) < 1e-12
        report minimum(d) == '2024-02-28' and maximum(d) == '2024-03-01'
        report mean(d) == '2024-02-29' and median(d) == '2024-02-29'
        report pmf(uniform('0001-01-01','0001-01-01'),'0001-01-01') == 100%
        report pmf(uniform('9999-12-31','9999-12-31'),'9999-12-31') == 100%
        let estimate = '2027-01-01' to '2027-01-11'
        report typeof estimate == "dist[date]"
        report quantile(estimate,5%) == '2027-01-01' and quantile(estimate,95%) == '2027-01-11'
        report median(estimate) == '2027-01-06' and mean(estimate) == '2027-01-06'
        report pmf(estimate,'2027-01-01') == pmf(estimate,'2027-01-11')
        let shifted = '1969-12-27' to '1970-01-06'
        report pmf(shifted,'1969-12-27') == pmf(estimate,'2027-01-01')
        report P(estimate < '2027-01-01') < 5% and P(estimate <= '2027-01-11') > 95%
        let x ~ d
        report x >= '2024-02-28' and x <= '2024-03-01'
    "#,
    );
    for source in [
        "report uniform('2024-01-02','2024-01-01')",
        "report uniform('2024-01-01',1)",
        "report '2024-01-01' to '2024-01-01'",
        "report '2024-01-02' to '2024-01-01'",
        "report '0001-01-01' to '0001-02-01'",
        "report '9999-12-01' to '9999-12-31'",
    ] {
        assert!(!error(source).is_empty(), "{source}");
    }
    for source in [
        "report uniform('2024-01-01','2024-12-31')",
        "report '2024-01-01' to '2024-12-31'",
    ] {
        let options = Options {
            limits: Limits {
                max_outcomes: 10,
                ..Limits::default()
            },
            ..Options::default()
        };
        assert_eq!(exec_raw(source, &options).unwrap_err().kind, ErrorKind::Limit);
    }
}

#[test]
fn now_is_one_snapshot_across_worlds_threads_calls_and_simulate() {
    let source = r#"
        fn anchor() { now }
        let captured = now
        let n ~ d6
        report captured == anchor() and captured == simulate { let k~d6; now }
        report date(now) == today
        report now
    "#;
    let stamp = dates::DateTime::parse("2027-01-01T00:15:00.123456789+01:00").unwrap();
    for threads in [1, 4] {
        for mode in [Mode::Enumerate, Mode::Sample { runs: 2100, seed: 7 }] {
            let out = exec_raw(
                source,
                &Options {
                    now: Some(stamp),
                    mode: Some(mode),
                    limits: Limits {
                        max_threads: threads,
                        ..Limits::default()
                    },
                    ..Options::default()
                },
            )
            .unwrap();
            assert_eq!(out.now, Some(stamp));
            assert_eq!(out.today, Some(stamp.day()));
            assert_eq!(out.reports[0].chance(), Some(1.0));
            assert_eq!(out.reports[1].chance(), Some(1.0));
            assert_eq!(out.reports[2].distribution(), vec![(Value::DateTime(stamp), 1.0)]);
        }
    }
    let out = exec_raw(
        "report now",
        &Options {
            today: Some(stamp.day()),
            ..Options::default()
        },
    )
    .unwrap();
    assert_eq!(out.now, dates::DateTime::new(stamp.day(), 0, 0, 0, 0));
    assert!(
        exec_raw(
            "report now",
            &Options {
                today: Some(stamp.day() + 1),
                now: Some(stamp),
                ..Options::default()
            }
        )
        .is_err()
    );
    assert!(error("report now").contains("host did not supply"));
    assert!(compile_error("report now()").contains("constant, not a function"));
    facts("let now=7; report now==7");
}
