mod common;
use common::*;
use probl_engine::{Options, value::Value};
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
        for r in out.reports {
            assert_eq!(r.distribution(), vec![(Value::Bool(true), 1.0)], "{source}");
        }
    }
}
#[test]
fn units_literals_construction_identity_and_dimensions() {
    facts(
        r#"
        let d: duration = '1h 30m'
        let v: squared_duration = '0.25s^2'
        report d == hours(1) + minutes(30) and d == seconds(5400)
        report milliseconds(1500) == '1.5s' and microseconds(1) == '1000ns'
        report nanoseconds(0.5) == '0.5ns'
        report days(1.5) == hours(36) and weeks(1) == days(7)
        report typeof d == "duration" and typeof v == "squared_duration"
        report '-1h 30m' == -d and d * 2 == '3h'
        report d / '1m' == 90 and '2s' * '2s' == '4s^2'
        report '4s^2' / '2s' == '2s' and '4s^2' / '1s^2' == 4
        report sqrt(v) == '0.5s' and '0.5s'^2 == v
        report '1s' != '1s^2' and '1s' != 1000000000
        report len(set('1m','60s',seconds(60))) == 1
        report ['1m':5]['60s'] == 5
        report ['2s','1s'].sort() == ['1s','2s']
        report abs('-1s') == '1s'
        report sum(['1s','2s']) == '3s'
        report sum([one_of(['1s','2s']),'2s']) >= '3s'
        report '9007199254740993ns' / 1 == '9007199254740993ns'
        report parse_duration("1h 30m") == d
        report parse_duration("0.25s^2") == v
        report match '60s' { '1m' => true, _ => false }
        let xs: list[duration or squared_duration] = [d,v]
        report len(xs) == 2
    "#,
    );
}
#[test]
fn division_and_remainder_split_matching_time_units() {
    facts(
        r#"
        let elapsed = '1d 2h 3m 4.5s'
        let whole_hours: int = elapsed div '1h'
        let rest: duration = elapsed mod '1h'
        report whole_hours == 26 and rest == '3m 4.5s'
        report whole_hours * '1h' + rest == elapsed
        report (rest div '1m') == 3 and (rest mod '1m') == '4.5s'
        report '90m' div '1h' == 1 and '90m' mod '1h' == '30m'
        report '-90m' div '1h' == -2 and '-90m' mod '1h' == '30m'
        report '90m' div '-1h' == -2 and '90m' mod '-1h' == '-30m'
        report '-90m' div '-1h' == 1 and '-90m' mod '-1h' == '-30m'
        report '0s' div '1h' == 0 and '2h' mod '1h' == '0s'
        report '0.75ns' div '0.5ns' == 1 and '0.75ns' mod '0.5ns' == '0.25ns'
        report '1ns' div '0.375ns' == 2 and '1ns' mod '0.375ns' == '0.25ns'
        report '-0.75ns' div '0.5ns' == -2 and '-0.75ns' mod '0.5ns' == '0.25ns'
        let squared: squared_duration = '5s^2' mod '2s^2'
        report '5s^2' div '2s^2' == 2 and squared == '1s^2'
        let huge = nanoseconds(10^400 + 1)
        report huge div '3ns' == (10^400 + 1) div 3 and huge mod '3ns' == '2ns'
        report ('2024-03-01' - '2024-02-28') div '1d' == 2
        let origin = '2027-01-04T00:00:00'
        let t = '2027-01-12T09:30:00'
        report t - ((t - origin) mod '1w') == '2027-01-11T00:00:00'
        let values: dist[int] = one_of(['30m', '90m']) div '1h'
        let remainders: dist[duration] = one_of(['30m', '90m']) mod '1h'
        report pmf(values, 0) == 50% and pmf(values, 1) == 50%
        report pmf(remainders, '30m') == 100%
        report try { '1s' div '0s' } catch DivisionByZero { 42 } == 42
        report try { '1s' mod '0s' } catch DivisionByZero { '2s' } == '2s'
        report try { '0.5ns' mod '0ns' } catch DivisionByZero { '2s' } == '2s'
    "#,
    );
    for source in [
        "report '1s' div '1s^2'",
        "report '1s^2' div '1s'",
        "report '1s' mod '1s^2'",
        "report '1s^2' mod '1s'",
        "report '1s' div 2",
        "report '1s' mod 2",
    ] {
        assert!(!error(source).is_empty(), "{source}");
    }
}
#[test]
fn date_differences_are_exact_durations_in_both_modes() {
    facts(
        r#"
        let start = '2024-02-28'
        let end = '2024-03-01'
        let elapsed: duration = end - start
        report typeof elapsed == "duration" and elapsed == '2d' and elapsed == hours(48)
        report elapsed != 2 and elapsed / '1d' == 2
        report start - end == '-2d' and start - start == '0s'
        report start + elapsed == end and end - elapsed == start
        report elapsed == '2024-03-01T00:00:00' - '2024-02-28T00:00:00'
        report '1970-01-01' - '1969-12-31' == '1d'
        let span = '9999-12-31' - '0001-01-01'
        report span == days(3652058) and span + '0001-01-01' == '9999-12-31'
        report '9999-12-31' + ('0001-01-01' - '9999-12-31') == '0001-01-01'
        let delays: dist[duration] = one_of([start, end]) - start
        report typeof delays == "dist[duration]" and pmf(delays, '0d') == 50% and pmf(delays, '2d') == 50%
        report pmf(end - one_of([start, end]), '2d') == 50%
        let arrival ~ one_of([start, end])
        report start + (arrival - start) == arrival
    "#,
    );
}
#[test]
fn datetime_arithmetic_checks_resolution_and_date_bounds() {
    facts(
        r#"
        let t = '2027-01-01T00:00:00.000000001'
        report t + '1s' == '2027-01-01T00:00:01.000000001'
        report '1h' + t == t.add_hours(1)
        report t - '2ns' == '2026-12-31T23:59:59.999999999'
        report (t + '1ns') - t == '1ns'
        report ('9999-12-31T23:59:59.999999999' - '0001-01-01T00:00:00') + '0001-01-01T00:00:00' == '9999-12-31T23:59:59.999999999'
        report '2024-02-29' + '24h' == '2024-03-01'
        report '2024-03-01' - days(1) == '2024-02-29'
    "#,
    );
    for source in [
        "report '2027-01-01T00:00:00' + '0.5ns'",
        "report '2027-01-01' + '1h'",
        "report '9999-12-31T23:59:59.999999999' + '1ns'",
        "report '0001-01-01T00:00:00' - '1ns'",
        "report '1s' + 1",
        "report '1s' + '1s^2'",
        "report '1s^2' < '1s'",
        "report '1s' / '0s'",
        "report sqrt('-1s^2')",
        "report sqrt('1s')",
    ] {
        assert!(!error(source).is_empty(), "{source}");
    }
}
#[test]
fn datetime_moments_preserve_nanoseconds_and_carry_spread_units() {
    facts(
        r#"
        let a='2027-01-01T00:00:00.000000001'
        let b=a+'1ns'
        report mean([a,b]) == a
        report mean([a,a+'2ns']) == b
        report mean(one_of([a:25%,a+'4ns':75%])) == a+'3ns'
        report sd([a,b]) == '0.5ns' and variance([a,b]) == '0.25ns^2'
        report sd(one_of([a,b])) == '0.5ns'
        report typeof sd([a,b]) == "duration" and typeof variance([a,b]) == "squared_duration"
        report variance([a,a+'2s']) / '1s^2' == 1
        report sd([a,a+'2s']) / '1ms' == 1000
        report mean(['1969-12-31T23:59:59.999999999','1970-01-01T00:00:00']) == '1969-12-31T23:59:59.999999999'
        report mean(['0001-01-01T00:00:00','9999-12-31T23:59:59.999999999']) == median(['0001-01-01T00:00:00','9999-12-31T23:59:59.999999999'])
        report mean([a]) == a and sd([a]) == '0s' and variance([a]) == '0s^2'
        report mean(['1s','3s']) == '2s'
        report mean(['0ns','1ns']) == '0.5ns'
        report median(['0ns','1ns']) == '0.5ns'
        report sd(['1s','3s']) == '1s' and variance(['1s','3s']) == '1s^2'
        report mean(['1s^2','3s^2']) == '2s^2' and sd(['1s^2','3s^2']) == '1s^2'
        let huge = nanoseconds(10^100)
        report mean([huge,huge+'2ns']) == huge+'1ns'
        report sd([huge,huge+'2ns']) == '1ns'
        report variance([huge,huge+'2ns']) == '1ns^2'
    "#,
    );
    for source in [
        "report mean(['2027-01-01T00:00:00','2027-01-01'])",
        "report sd(['1s','1s^2'])",
        "report variance(['1s^2','2s^2'])",
    ] {
        assert!(!error(source).is_empty(), "{source}");
    }
}

#[test]
fn time_values_obey_integer_and_work_limits() {
    use probl_engine::{Limits, error::ErrorKind};
    let options = Options {
        limits: Limits {
            max_integer_bits: 64,
            ..Limits::default()
        },
        ..Options::default()
    };
    for source in [
        "report nanoseconds(1e100)",
        "report '1ns' * 1e100",
        "report parse_duration(\"18446744073709551616ns\")",
        "report '18446744073709551616ns'",
        "report '9999-12-31T00:00:00' - '0001-01-01T00:00:00'",
        "report '9999-12-31' - '0001-01-01'",
        "report '1ns' div '0.00000000000000000001ns'",
    ] {
        assert_eq!(
            exec_raw(source, &options).unwrap_err().kind,
            ErrorKind::Limit,
            "{source}"
        );
    }
    // Mean must not fail because an unrequested variance needs more bits.
    assert!(exec_raw("report mean(['0ns','18446744073709551615ns'])", &options).is_ok());
    let options = Options {
        limits: Limits {
            max_work: 100,
            ..Limits::default()
        },
        ..Options::default()
    };
    assert_eq!(
        exec_raw("report sd(['1ns','2ns'])", &options).unwrap_err().kind,
        ErrorKind::Limit
    );
}
#[test]
fn invalid_units_and_display_round_trips() {
    for text in [
        "1", "1month", "1year", "1s^3", "1s 1s^2", "1h -1m", "1ss", "NaNs", "infs", ".5s", "1.s",
    ] {
        assert!(!compile_error(&format!("report '{text}'")).is_empty(), "{text}");
    }
    for text in [
        "1h 30m",
        "-0.5ns",
        "0.25s^2",
        "0s",
        "9007199254740993ns",
        "0.3333333333333333ns",
        "1.23456789ms^2",
    ] {
        let value = distribution(&format!("report ['{text}']"))[0].0.clone();
        assert_eq!(distribution(&format!("report {value}"))[0].0, value, "{text}");
    }
}

#[test]
fn list_means_use_exact_counts_across_the_full_calendar() {
    use probl_engine::dates::DateTime;
    let a = DateTime::parse("0001-01-01T00:00:00").unwrap();
    let b = DateTime::parse("5000-06-12T03:04:05.000000001").unwrap();
    let c = DateTime::parse("9999-12-31T23:59:59.999999999").unwrap();
    let sum = a.total_nanos() + 3 * b.total_nanos() + 3 * c.total_nanos();
    let mean = sum.div_euclid(7) + i128::from(2 * sum.rem_euclid(7) > 7);
    let expected = DateTime::from_total_nanos(mean).unwrap().format();
    facts(&format!(
        "let a='{}'; let b='{}'; let c='{}'; report mean([a,b,b,b,c,c,c]) == '{expected}'; report mean([c,b,a,c,b,c,b]) == '{expected}'",
        a.format(),
        b.format(),
        c.format()
    ));
}
